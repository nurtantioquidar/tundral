Imports Shop.Services

Module Program
    Sub Main()
        Dim service As New UserService()
        Greet(service.Register("ada").Name)
    End Sub

    ' WHY: keep the console output stable for tests
    Private Sub Greet(name As String)
        Console.WriteLine("Hello " & name)
    End Sub
End Module
